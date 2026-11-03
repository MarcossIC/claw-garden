# retire-legacy-kinds — Exploración

| Objetivo | Tamaño | Depende de | Estado |
|---|---|---|---|
| 1 | M–L, con puerta | — | exploración (borrador) |

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

**Los datos guardan los nombres.** `project_delete_row.tbl` graba el nombre de la tabla como dato (`secrets.rs:352`, `:359-368`), así que un diario viejo puede nombrar estas tablas al deshacer o purgar.

## Enfoques posibles

| Enfoque | A favor | En contra |
|---|---|---|
| **A. Migrar las filas que haya a `memory` y borrar las tablas** | Quita todo el código. Ninguna fila se pierde | Migración con su test sembrado. Una fila migrada cambia de id y hay que seguir sus referencias |
| B. Dejar de buscarlas y conservar tablas y código | Sin migración. Saca el costo de cada búsqueda | Si hay filas, las esconde, y `config.yaml` lo prohíbe. Tampoco saca el código |
| C. Borrar el código y dejar las tablas | Sin migración | Si hay filas, quedan huérfanas e inalcanzables, y es lo mismo que esconderlas |
| D. Dejarlo como está | Sin riesgo | Cada refactor del almacenamiento paga por dos tablas sin escritor |

**Recomendado: A, con puerta.** Una base de otra persona puede tener filas aunque la de quien mide no las tenga, así que el camino sin pérdida hace falta en cualquier caso.

## Alcance candidato

1. **Puerta, sin código:** contar filas vivas y borradas en las dos tablas en cada perfil disponible, con autorización para abrirlos. `clawg export` ya informa `legacy_rows`. Si aparecen filas en número, se revisa qué son antes de seguir.
2. Migración nueva: cada fila viva pasa a `memory`, con su vector si lo tiene. Una fila que un borrado de proyecto dejó en su diario (`project_delete_row`) tampoco se puede perder en silencio, porque deshacer descarta con un `warn` un `tbl` que no conoce (`crates/clawg-core/src/memory/project_delete.rs:778-781`). Hay dos salidas: migrarla a `memory` con su `deleted_at` y reescribir en la misma transacción su entrada del diario (`tbl` = `memory` y `row_id` = el id nuevo), o, si existe alguna, abortar la actualización sin modificar la base y decir por qué. Después se borran las dos tablas y sus `vec0`. Test con una base sembrada en la versión anterior, según `config.yaml`, y un test que deshace un proyecto borrado antes de la migración. El snapshot `pre-migration` es la vuelta atrás.
3. Quitar los dos tipos de búsqueda, mantenimiento, proyectos, purga y exportación, y unificar `TableKind` y `MemoryKind`.
4. API, CLI y MCP: un `kind` viejo recibe un error que lo explica, no un 404 mudo. Bendecir los dos contratos.

### No-alcance

- Cambiar cómo se guardan o clasifican las memorias normales.
- Una función nueva de habilidades o trayectorias. Si vuelve, vuelve con su propio cambio y su propio escritor.

## Decisiones abiertas

1. A qué `memory_type` va una fila migrada. Una trayectoria y una habilidad no encajan igual en los tipos actuales.
2. Cuál de las dos salidas del punto 2 del alcance se elige para las filas de `project_delete_row`, y qué pasa con cualquier otra referencia a un id viejo.
3. Si una fila ya borrada que no está en ningún diario se migra para que `restore` la siga trayendo, o se conserva sólo en el snapshot.
4. Si el `kind` viejo se sigue aceptando por una versión, con un aviso, para clientes anteriores.

## Cómo se mide

- **Test en rojo:** una base en la versión anterior con filas de los dos tipos abre con la versión nueva, y cada fila viva se encuentra por búsqueda.
- **Test en rojo:** un proyecto con filas de los dos tipos, borrado antes de la migración, se deshace con la versión nueva y recupera esas filas. Si la decisión 2 elige abortar, la actualización falla con esa base y la deja intacta.
- Una búsqueda hace dos KNN y dos consultas de elegibles menos, visible en `search.duration`.
- `rag-eval --diff` con el modelo real sin casos movidos: el golden set no tiene filas de estos tipos.

## Riesgos

- Es la primera migración que borra tablas con datos posibles. Si la cuenta de un perfil real no da cero, la decisión 1 pasa a ser la importante.
- `doctor` y las invariantes tienen que seguir pasando sobre una base recién migrada.

## Interacciones

- **search-arm-timing:** deja de medir dos brazos.
- **knn-prefilter:** su no-alcance excluye estas tablas, y esta retirada lo confirma.
- **store-boundary y storage-write-side:** tienen menos superficie si este cambio va primero.
