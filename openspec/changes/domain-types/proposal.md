# domain-types — Propuesta

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| transversal (sirve al 1: guardar y clasificar) | L, por tandas; este cambio aplica la primera | core-use-cases (archivado) | ver [`state.yaml`](state.yaml) |

## Problema

El tipo y el estado de una memoria viajan como `String` desde la columna hasta el cliente, y cada decisión los compara con un literal. Un literal mal escrito compila y decide mal sin que nada lo note. Detalle en la [exploración](exploration.md#problema-verificado-contra-el-código); las líneas que siguen son las de `develop` al escribir esto:

1. **Las filas leídas llevan texto:** `RawHit` (`crates/clawg-core/src/memory/retrieval/score.rs:17`, `:19`), `RankedHit` (`search.rs:145`, `:147`), `MemoryAdminRow` (`listing.rs:36`, `:40`), `ResolvedRow` y `PairRow` (`dispute.rs:114`, `:132`, `:135`), `RestoredRow` (`admin.rs:327`), `MemoryLine` (`portability/format.rs:99`, `:117`) y las filas de `relocate.rs`, `project_delete.rs`, `topic.rs`, `pieces.rs`, `undo.rs` e `import.rs`.
2. **Las decisiones comparan literales:** la puerta de estado del ranking (`score.rs:283`), la tasa de decaimiento por tipo (`score.rs:213`), las reglas de tier (`store/tier.rs:34-60`), la marca de una preferencia (`provenance.rs:58`), el reemplazo de una fila (`dispute.rs:140`, `admin.rs:551`, `undo.rs:461`), la vida de una fila importada (`import.rs:331`, `:1339`, `translate.rs:745`) y los tipos protegidos de un borrado de proyecto (`project_delete.rs:467`).
3. **Del otro lado del daemon, igual:** el cliente lee el JSON en `String` (`crates/clawg-cli/src/client.rs:216`, `:270`, `:275`, `:528`, `:562`, `:616`) y el puente decide con literales (`mcp/tools.rs:197`, `:271`, `:416-428`, `:532`; `undelete.rs:20-31`).

`MemoryType` ya existe (`crates/clawg-core/src/memory/domain/record.rs:95`), pero sólo para lo que entra por la API: lo que se lee de la base vuelve a ser texto. Para el estado no hay tipo.

## Enfoque

El **A** de la exploración: lo que se lee de la base es un `Stored<T>`, con una variante `Other(String)` para un nombre que este código no conoce; lo que entra por la API sigue siendo estricto. Son dos direcciones: lo que entra se rechaza, lo guardado se conserva.

- `MemoryStatus` (`Active`, `Superseded`, `Disputed`) junto a `MemoryType`, en `memory/domain/record.rs`.
- `Stored<T>`, con los alias `StoredType` y `StoredStatus`: lee cualquier texto sin fallar, se escribe en SQLite y en JSON exactamente como se leyó, y se compara por su texto con un `T`.
- Las decisiones hacen `match` sobre el enum, con un brazo explícito para `None` (el nombre desconocido) que repite lo que hace hoy el comodín `_`.

Las tres decisiones abiertas de la exploración y las que surgieron al aplicar están en el [diseño](design.md).

## Alcance

La tanda 1 de la exploración, de punta a punta:

1. **Los tipos** en `memory/domain/record.rs`, con `ToSql`, `FromSql`, `Serialize`, `Deserialize`, `Display` y `Debug` del texto, reexportados en `clawg_core::memory`.
2. **El núcleo:** cada fila leída de `memory` lleva `StoredType` y `StoredStatus`; cada decisión sobre ellos es un `match` o una comparación con el enum; `compute_tier` y `retier` reciben `Option<MemoryType>`; el evento del outbox lleva `Option<StoredType>`.
3. **El daemon:** el filtro de estado de `GET /v1/memories` se parsea con `MemoryStatus` (mismo texto de error); los filtros de tipo de un listado y de un movimiento aceptan cualquier nombre, como hoy.
4. **El cliente y el puente:** las respuestas que lee `DaemonClient` llevan los mismos tipos, y `mcp/tools.rs`, `undelete.rs` y `memories.rs` deciden con el enum.
5. **El SQL:** los literales siguen en el texto de las consultas, y un test de convenciones comprueba que cada uno es un nombre conocido.

## Fuera de alcance

- Las tandas 2 a 5 de la exploración: ids por tabla, `ProjectId` y `AgentId`, structs de parámetros y el predicado de alcance.
- Lo que entra por la CLI y por el puente MCP (`--memory-type`, el `memory_type` de `save_memory` y de `update_memory`): sigue siendo texto y lo valida el daemon, que devuelve el mismo error de hoy.
- Las claves de los reportes (`DeleteReport::by_type`, `ImportReport::by_type`, `MemoryStats`) y las etiquetas de las métricas de sesión: son texto agrupado, no una fila.
- `eval_api`: sus firmas públicas no cambian; convierte en su borde.
- Cambiar el esquema, los valores guardados, un cuerpo de respuesta, un mensaje de error, el ranking o cualquier conducta.

## Datos guardados

Ninguno. Lo que se escribe en `memory`, en `ingest_outbox`, en los diarios de import y movimiento y en el JSON de la API y de un export queda idéntico byte a byte: `Stored<T>` escribe y serializa su texto, sea `Known` u `Other`. No hay migración ni vuelta atrás: una versión anterior lee lo mismo que lee hoy.

Lo delicado es leer: ningún valor desconocido puede hacer fallar una lectura ni esconder una fila. `FromSql` de `Stored<T>` acepta cualquier texto, y falla sólo donde fallaba el `String` de hoy (un valor que no es texto).

## Cómo se mide

- **Tests en rojo** ([spec](spec.md)): los de `Stored<T>` y `MemoryStatus` en `record.rs`, y los de conservación en `search.rs`, `listing.rs` y `events.rs`, que siembran un `memory_type` `observation` (el `DEFAULT` de `V1__init.sql`) y un `status` `archived`. No compilan antes del cambio: los tipos no existen.
- **La búsqueda no cambia:** un `status` fuera de `active` y `disputed` sigue sin entrar en ella; un tipo desconocido con estado vivo sí entra y rankea con la tasa de reserva.
- **Nada de lo que ve un cliente cambia:** `crates/clawg-cli/tests/api.rs` pasa sin editar ninguna aserción, y los dos contratos del agente pasan sin `CLAWG_BLESS`.
- **El SQL:** `every_status_and_type_the_sql_compares_is_one_the_code_knows` (`crates/clawg-cli/tests/conventions.rs`).
- **Sin métrica de `rag-eval`:** el ranking no cambia; el `match` de `lambda_for` y la puerta de estado dan los mismos valores que los literales, y lo fijan los tests de `score.rs`.
