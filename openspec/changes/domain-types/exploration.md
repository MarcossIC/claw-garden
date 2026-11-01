# domain-types — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal | L, por tandas independientes | core-use-cases | exploración (borrador) |

**Tesis:** el dominio viaja como `String` e `i64`. El tipo de una memoria se valida y se descarta, el estado se compara con literales de SQL, y un id de una tabla se puede pasar donde va el de otra. Son errores que el compilador podría impedir, y conviene resolverlo antes de definir firmas de almacenamiento, para que esas firmas nazcan bien.

## Problema (verificado contra el código)

Conteos en `clawg-core`, producción, sin `eval_api` (por patrón de texto, para dar el orden de magnitud):

| Qué | Cuántos |
|---|---|
| `memory_type` como `String` u `Option<String>` | 17 |
| `status` como `String` u `Option<String>` | 17 |
| Campos o parámetros de agente como texto | 67 |
| Campos o parámetros de proyecto como texto | 158 |
| Ids como `i64` | 142, más 23 listas |
| Literales `'active'`, `'superseded'`, `'disputed'` | 37, 25 y 40 |
| Funciones con 7 o más parámetros (todo el workspace) | 38 |

Ejemplos concretos:

- `RankedHit` guarda `memory_type: String` y `status: String` (`crates/clawg-core/src/memory/search.rs:127`, `:129`), aunque `MemoryType` existe (`store_insert.rs:25`).
- `crates/clawg-cli/src/daemon/api.rs:1338-1340` parsea `MemoryType` y descarta el resultado, y `:1404` pasa el texto original.
- El comentario de `search.rs:1368-1373` advierte que un id de `memory` puede coincidir con uno de otra tabla y que mezclarlos pierde candidatos. Lo cuida una convención, no un tipo.
- `knn_one` tiene 9 parámetros (`search.rs:548`), `build_memory_prompt` 11, y `run` recibe `proj: &Option<String>` (`search.rs:580`), que en Rust se escribe `Option<&str>`.
- `TableKind` (`record.rs:10`) y `MemoryKind` (`admin.rs:37`) son el mismo enum dos veces. Lo unifica retire-legacy-kinds, si va primero.

**Precedentes en el propio código:** `TimestampMs` (`clock.rs:35`), `OutboxState` (`outbox.rs:35`) y `Reach` (`sharing.rs:43`). `EmbeddingIdentity` incluso explica por qué usa campos con nombre y no dos `String` seguidos (`db.rs:26-28`).

## El punto delicado: leer no puede perder filas

Hoy, una fila con un `status` inesperado llega como texto y sigue siendo legible. Con un enum estricto, `FromSql` falla, y esa fila desaparece de una búsqueda o la rompe. `config.yaml` lo prohíbe: «No change may lose, hide or destroy stored memories». Una base puede tener valores que este código no conoce, escritos por otra versión o por un import.

| Opción para un valor desconocido | A favor | En contra |
|---|---|---|
| **A. Variante `Other(String)` en lo que se lee de la base** | Ninguna fila se pierde, y el valor se sigue mostrando | Cada `match` tiene que decidir qué hace con ella |
| B. Fallar la lectura de esa fila | Tipos sin caso raro | Esconde memorias. Rechazada |
| C. Mapear lo desconocido a un valor por defecto | Código simple | Reescribe en silencio lo que dice la fila. Rechazada |

**Recomendado: A para los valores que vienen de la base**, y un tipo estricto para lo que entra por la API. Son dos direcciones distintas: lo que entra se rechaza, lo guardado se conserva.

## Alcance candidato, en tandas que se pueden hacer por separado

1. `MemoryType` y `MemoryStatus` de punta a punta, con `ToSql` y `FromSql` y las constantes de SQL reunidas en un lugar. El serializado JSON no cambia.
2. Ids por tabla: `MemoryId` y, si sobreviven, los de las otras tablas.
3. `ProjectId` y `AgentId`.
4. Structs de parámetros para las funciones más anchas (`KnnQuery` y los `build_*_prompt`), con lo que se caen los `#[expect(clippy::too_many_arguments)]` de clippy-enforced.
5. El predicado de alcance: `agents: &str` lleva un JSON armado por `reach_bind` (`search.rs:65-71`). Un tipo propio evita pasar un texto cualquiera.

### No-alcance

- Cambiar el esquema o los valores guardados: lo que se escribe en la base y en el JSON queda idéntico byte a byte.
- Cambiar ranking o conducta.
- Optimizar clones, por ejemplo con `Arc<str>`. Sólo si una medición lo pide.

## Decisiones abiertas

1. Cómo se escriben los literales de SQL con el enum: constantes en `concat!`, como hacen `reach_predicate!` y `memory_project_predicate!`, o parámetros enlazados. Un literal ayuda al planificador con los índices parciales, y eso se comprueba con los tests de plan.
2. Si `ProjectId` valida su formato al construirse (`is_project_id` ya existe) o sólo distingue el tipo.
3. Si cada tabla tiene su tipo de id, o `MemoryId` más un `TableKind` donde hace falta.

## Cómo se mide

- **Test en rojo:** una fila sembrada con un `status` y un `memory_type` que el código no conoce sigue apareciendo en una búsqueda y en un listado.
- El JSON de `crates/clawg-cli/tests/api.rs` y los contratos del agente no cambian.
- `rag-eval --diff` con el embedder falso sin casos movidos.
- `perf-bench` sin regresión: los newtypes de `String` no deberían costar nada nuevo.

## Riesgos

- El cambio toca muchas firmas y choca con todo cambio abierto. Por eso va en tandas.
- Una conversión de `RawHit` a tipos estrictos en el hilo de la conexión suma trabajo a ese hilo. Es poco, pero la cola de read-connections lo paga.

## Interacciones

- **core-use-cases:** los tipos validados entran por ahí.
- **storage-read-side y storage-write-side:** sus firmas usan estos tipos, por eso este cambio va antes.
- **retire-legacy-kinds:** si va primero, hay menos tipos de id y un solo enum de tipo.
