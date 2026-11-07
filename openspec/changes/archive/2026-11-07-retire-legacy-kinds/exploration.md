# retire-legacy-kinds — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M–L, con puerta | — | exploración cerrada: la puerta pasó y las cuatro decisiones están tomadas |

**Tesis:** `episodic_trajectory` y `procedural_skill` no reciben filas nuevas, pero cada búsqueda las sigue consultando y el código las carga en todas partes. Son el sobrante de cuando el producto era un agente. Conviene sacarlas antes de abstraer el almacenamiento: no tiene sentido diseñar un puerto para lo que se va a borrar. Por la regla de `config.yaml` sobre la base del usuario, nada se borra sin saber primero qué filas hay y sin un camino de vuelta.

## Problema (verificado contra el código)

**Nadie escribe.**

- `insert_episodic_trajectory` (`crates/clawg-core/src/memory/store_insert.rs:660`) es `pub`, y todos sus llamadores están en módulos de test. Por ser `pub`, el lint de `dead_code` no lo ve.
- `promote_to_procedural_skill` (`store_insert.rs:760`) está bajo `#[cfg(test)]` (`:745`).
- `crates/clawg-core/src/memory/portability/export.rs:41-42` ya lo dice: «nothing writes any more and which do not travel». knn-prefilter las da por vacías en su no-alcance.
- Lo que queda en producción sobre esas tablas modifica filas existentes: `admin.rs:263-266`, `admin.rs:372-374`, `merge.rs:510-513`.

Queda sin descartar SQL armado por nombre de tabla, que una búsqueda literal no ve. Por ejemplo, `secrets.rs:370` y `:388` arman su consulta con `format!`, aunque sólo para borrar.

**Todos pagan.**

- `SearchScope::all_default` pide `episodic: Some(5)` y `procedural: Some(5)` (`search.rs:38-47`): dos KNN y, como vuelven vacíos, dos consultas de elegibles (`search.rs:633-642`), en cada búsqueda.
- Mantenimiento recorre las tablas (`maintenance.rs:70`, `:261-262`), con un comentario largo sobre un caso que hoy no puede pasar (`:56-69`).
- Las consultan o las cargan: `project.rs:43`, `:789-790`; `project_delete.rs:307`, `:356-357`, `:502`; `merge.rs:274`; `secrets.rs:362-363`; `inspect.rs:395-396`; `workspace.rs:277`, `:442`; `export.rs:219-220`; `doctor/checks.rs:803`.
- Unas 400 líneas de producción las mencionan (conteo por palabra, cota superior). `TableKind` (`record.rs:10`) y `MemoryKind` (`admin.rs:37`) son dos enums con las mismas tres variantes.

**Son visibles por fuera.**

- API: `parse_kind` (`crates/clawg-cli/src/daemon/api.rs:217-218`).
- CLI: `KindArg` (`crates/clawg-cli/src/cli.rs:927-928`).
- MCP: el contrato del agente las lista en cuatro herramientas (`crates/clawg-cli/src/mcp/agent_contract.json:59`, `:86`, `:154`, `:541`), así que quitarlas cambia lo que ve el modelo y pide bendecir los dos contratos.
- Specs: 42 líneas en 12 archivos de `openspec/specs/` mencionan estos tipos, aunque algunas pueden ser sólo contexto.

**Los datos guardan los nombres.** Tres diarios guardan el id de una fila de estas tablas como dato, así que uno viejo puede nombrarlas al deshacer o purgar:

- `project_delete_row.tbl`, del borrado de un proyecto (`secrets.rs:352`, `:359-368`).
- `project_merge_change`, con `kind` `trajectory` o `skill` y el id en `a`, de la unión de dos proyectos: lo escribe `merge.rs:274` y lo deshace `merge.rs:507-513`.
- `project_rekey.tbl`, de la renumeración de proyectos. Se escribe en producción (`db.rs:1953`, `project.rs:888`), pero sólo lo leen los tests y la receta del comentario de `V9__project.sql`.

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Migrar las filas que haya a `memory` y borrar las tablas** | Quita todo el código. Ninguna fila se pierde | Migración con su test sembrado. Una fila migrada cambia de id y hay que seguir sus referencias |
| B. Dejar de buscarlas y conservar tablas y código | Sin migración. Saca el costo de cada búsqueda | Si hay filas, las esconde, y `config.yaml` lo prohíbe. Tampoco saca el código |
| C. Borrar el código y dejar las tablas | Sin migración | Si hay filas, quedan huérfanas e inalcanzables, y es lo mismo que esconderlas |
| D. Dejarlo como está | Sin riesgo | Cada refactor del almacenamiento paga por dos tablas sin escritor |

**Recomendado: A, con puerta.** Una base de otra persona puede tener filas aunque la de quien mide no las tenga, así que el camino sin pérdida hace falta en cualquier caso.

## Alcance candidato

1. **Puerta, sin código:** contar filas vivas y borradas en las dos tablas en cada perfil disponible, con autorización para abrirlos. `clawg export` ya informa `legacy_rows`. Si aparecen filas en número, se revisa qué son antes de seguir. Hecho: ver [Resultado de la puerta](#resultado-de-la-puerta-2026-11-03).
2. Migración nueva: cada fila pasa a `memory`. Una fila viva pasa con su vector si lo tiene, y una borrada pasa con su `deleted_at` y sin vector (ver la decisión 3). Una fila que un borrado de proyecto dejó en su diario (`project_delete_row`) tampoco se puede perder en silencio, porque deshacer descarta con un `warn` un `tbl` que no conoce (`crates/clawg-core/src/memory/project_delete.rs:778-781`): se migra a `memory` con su `deleted_at`, y en la misma transacción se reescribe todo lo que nombra su id viejo (ver la decisión 2). Después se borran las dos tablas y sus `vec0`. Test con una base sembrada en la versión anterior, según `config.yaml`, y un test que deshace un proyecto borrado antes de la migración. El snapshot `pre-migration` es la vuelta atrás.
3. Quitar los dos tipos de búsqueda, mantenimiento, proyectos, purga y exportación, y unificar `TableKind` y `MemoryKind`.
4. API, CLI y MCP: desde la primera versión, un `kind` viejo recibe un error que lo explica, no un 404 mudo ni el rechazo genérico de `clap` (ver la decisión 4). Bendecir los dos contratos.

### No-alcance

- Cambiar cómo se guardan o clasifican las memorias normales.
- Una función nueva de habilidades o trayectorias. Si vuelve, vuelve con su propio cambio y su propio escritor.

## Resultado de la puerta (2026-11-03)

Contado con `sqlite3 -readonly` sobre el `memory.db` de los perfiles `prod` y `dev`, con `count(*)` en las dos tablas y en `project_delete_row` filtrado por `tbl`:

| Perfil | `episodic_trajectory` | `procedural_skill` | Entradas del diario que las nombran |
|---|---:|---:|---:|
| `prod` | 0 | 0 | 0 |
| `dev` | 0 | 0 | 0 |

**La puerta pasa:** no hay filas vivas ni borradas, y ningún diario nombra estas tablas. El alcance no cambia, porque la base de otra persona puede tener filas: la migración sin pérdida se escribe igual.

## Decisiones

- [x] **A qué `memory_type` va una fila migrada.** Las dos a `semantic_fact`, con su tabla de origen anotada en la fila para distinguirlas después.
  - Una trayectoria como `session_summary` encajaría con lo que es, pero el resumen más nuevo de un proyecto abre la sesión siguiente bajo LAST SESSION: en un proyecto sin resúmenes, una trayectoria de hace meses aparecería como la última sesión.
  - Dos tipos nuevos sumarían un octavo `MemoryType` que nadie escribe, lo contrario de lo que busca este cambio.
  - Lo que se pierde: hoy las cinco habilidades más usadas se imprimen en el workspace (`crates/clawg-core/src/memory/workspace.rs:459`); como `semantic_fact` sólo salen por búsqueda. En las dos bases contadas no hay ninguna.
- [x] **Qué pasa con lo que nombra el id viejo de una fila migrada.** Se reescribe al id nuevo en la misma transacción que la migra: las entradas de los tres diarios de «Los datos guardan los nombres» y las referencias entre filas de estas tablas (`superseded_by`, `disputed_against`, los `source_trajectory_ids` de una habilidad). Si algo falla, la migración entera se deshace.
  - Deshacer un borrado o una unión de proyectos hechos antes de la actualización sigue trayendo esas filas, ya como `semantic_fact`.
  - Se descartó frenar la actualización cuando un diario las nombra: esa base no abriría con la versión nueva, sus agentes quedarían sin memoria hasta reinstalar la anterior, deshacer o purgar esa operación y volver a actualizar, y todo para evitar un caso que la reescritura resuelve.
- [x] **Qué pasa con una fila ya borrada que no está en ningún diario.** Se migra a `memory` como `semantic_fact` con su `deleted_at`, en la misma pasada que las vivas, sin vector: se le calcula al restaurarla, igual que a cualquier fila borrada de `memory`.
  - Hoy no la puede traer `restore`, que sólo restaura filas de `memory` (`crates/clawg-core/src/memory/admin.rs:695-697`), ni la puede borrar `clawg purge <ID>`, que sólo borra filas de `memory`. `clawg purge --project-delete N` sí entra en estas tablas, pero sólo para las filas que anotó su diario (`docs/reference/commands.md:17-18`). Una vez migrada, se puede hacer cualquiera de las dos cosas.
  - Se descartó dejarla sólo en el snapshot `pre-migration`: de las copias automáticas se guardan cinco, y de las `pre-migration` sólo se asegura la más nueva (`crates/clawg-core/src/memory/backup.rs:298-325`). Unas pocas copias más tarde la fila se perdería, y mientras siguiera ahí, recuperarla obligaría a volver la base entera a ese día. `config.yaml` no deja perder memorias guardadas, y una fila borrada sigue guardada.
- [x] **Qué recibe un cliente que pide un `kind` viejo.** Un error que lo explica desde la primera versión que retira estos tipos, en la API, la CLI y el MCP. Dice que `episodic` y `procedural` ya no existen, que sus filas pasaron a `memory` como `semantic_fact` con otro id, y que se encuentran con `search_memory`. No hay versión de tolerancia.
  - Quien puede mandarlo: un bridge de la versión anterior que el editor sigue teniendo abierto después de actualizar, cuyo contrato todavía ofrece esos valores, o un script que los use. El bridge viejo sólo los manda si el agente los elige; sin `kind` no manda ninguno (`crates/clawg-cli/src/client.rs:1299-1306`). Y le pasa al agente el texto del error del daemon (`client.rs:64-66`), así que el agente lee la explicación y vuelve a intentar con `memory`.
  - Se descartó aceptarlos una versión más, contestando vacío con un aviso: dejaría los dos nombres en la API y la CLI, pediría un segundo cambio para sacarlos, y leer una fila migrada por su id viejo contestaría «no existe» aunque la fila exista en `memory` con otro id. Sin escritor y sin filas en las bases contadas, una lista vacía y un error informan lo mismo, pero sólo el error dice dónde quedó cada fila.

## Cómo se mide

- **Test en rojo:** una base en la versión anterior con filas de los dos tipos abre con la versión nueva, y cada fila viva se encuentra por búsqueda.
- **Test en rojo:** un proyecto con filas de los dos tipos, borrado antes de la migración, se deshace con la versión nueva y recupera esas filas.
- **Test en rojo:** una unión de proyectos con filas de los dos tipos, hecha antes de la migración, se deshace con la versión nueva y esas filas vuelven al proyecto de origen.
- **Test en rojo:** una fila de cada tipo, borrada a mano antes de la migración, se recupera con `restore` en la versión nueva, y también se puede purgar.
- **Test en rojo:** pedir `episodic` o `procedural` a la API, la CLI o el MCP da el error que explica adónde fueron esas filas.
- Una búsqueda hace dos KNN y dos consultas de elegibles menos, visible en `search.duration`.
- `rag-eval --diff` con el modelo real sin casos movidos: el golden set no tiene filas de estos tipos.

## Riesgos

- Es la primera migración que borra tablas con datos posibles. Las dos bases contadas dan cero, así que ninguna fila real orienta la decisión 1 ni prueba la migración: sólo la prueba el test sembrado.
- `doctor` y las invariantes tienen que seguir pasando sobre una base recién migrada.

## Interacciones

- **search-arm-timing:** deja de medir dos brazos.
- **knn-prefilter:** su no-alcance excluye estas tablas, y esta retirada lo confirma.
- **store-boundary y storage-write-side:** tienen menos superficie si este cambio va primero.
