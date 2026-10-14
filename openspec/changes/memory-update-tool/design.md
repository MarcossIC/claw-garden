# memory-update-tool — Diseño

Las decisiones de producto (qué campos, qué scope, el tier, el re-guardado) están en la [propuesta](proposal.md). Acá van las de implementación.

## D1. Un cambio, no dos endpoints

| Opción | A favor | En contra |
|---|---|---|
| **a. Ampliar `PATCH /v1/memories/{kind}/{id}` a `{importance?, memory_type?}`** | Un endpoint, un camino en el core; el `?agent=` y el 404 de visibility-policy ya están | El cuerpo deja de tener un campo obligatorio, y hay que rechazar el vacío a mano |
| b. Un endpoint por campo (`/importance`, `/type`) | Cada uno trivial | Cambiar los dos es dos transacciones, y el tier de R2 se calcula dos veces con un estado intermedio que nadie pidió |
| c. `POST /v1/memories/{kind}/{id}/update` | Nombre explícito | Duplica la ruta que `PATCH` ya expresa |

**Elegida a.** El tier depende de los dos campos juntos (R2), así que tienen que entrar en el mismo cambio. `set_importance_core` pasa a ser `update_memory_core` y `DaemonClient::set_importance`, `DaemonClient::update`: el único llamador de los dos es el propio daemon y sus tests (`rg -n set_importance` lo confirma), así que no queda un alias.

## D2. Validación pura antes de la base

`MemoryPatch::new(kind, importance, memory_type) -> Result<MemoryPatch, PatchError>` en `admin.rs`:

- `PatchError` con `thiserror`: `Empty`, `ImportanceOutOfScale(f64)`, `UnknownType(String)`, `TypeOnlyOnMemory(MemoryKind)`. El daemon la mapea a 400 con su `Display`; el puente la recibe como cualquier error del daemon.
- `memory_type` sale de `MemoryType::from_db_str` (`store_insert.rs:55`), el mismo mapeo del save. A diferencia del save, un tipo desconocido es error y no `SemanticFact` (spec R1).
- El cuerpo HTTP lleva `#[serde(deny_unknown_fields)]`: `{"importanse": 3}` sería, si no, un cuerpo vacío válido para serde que recién rechazaría `Empty`, con un mensaje que no dice qué estaba mal escrito.

Así el core recibe un `MemoryPatch` que ya es válido por construcción, y los tests de R1 no abren una base.

## D3. La regla del tier, pura y aparte de `compute_tier`

`retier(current, access_count, before: (f64, &str), after: (f64, &str)) -> i64` en `tier.rs`, con los tres casos de R2. Se apoya en `compute_tier(3, ...)` para el tier ganado.

| Opción | A favor | En contra |
|---|---|---|
| **a. Función nueva; `compute_tier` sigue sólo de subida** | El invariante que documenta `tier.rs` (un acceso nunca baja un tier) no se toca; la baja que pide alguien tiene nombre propio | Una función más |
| b. Darle a `compute_tier` un modo que baja | Un lugar | `compute_tier` es la UDF que corre en cada flush de accesos y en cada refuerzo (`access.rs:207`, `store_insert.rs:314`); un parámetro de más ahí es la forma de que un acceso termine bajando un tier |
| c. UDF nueva y un solo `UPDATE` | Sin leer la fila antes | Otra UDF que registrar en `open` y en `new_for_test`, sólo para un camino que corre a pedido |

**Elegida a.** `update_memory_core` lee la fila (`tier`, `importance`, tipo efectivo, `access_count`) y escribe dentro de la misma transacción del `conn.call`. El comentario de `admin.rs:120-125` evitaba leer y escribir desde Rust por la ventana entre las dos sentencias; con las dos dentro de una transacción en la única conexión serializada (`tokio_rusqlite`), no hay lector que vea el estado intermedio. El comentario se reemplaza por éste.

## D4. Lo que escribe y lo que devuelve

Un `UPDATE` por tabla, con constantes como las de hoy (nada de interpolar el nombre de la tabla):

```sql
UPDATE memory
SET importance = ?1, memory_type = ?2, tier = ?3,
    last_modified_at = ?4, update_count = update_count + 1
WHERE id = ?5
```

Los campos que no se pidieron se escriben con el valor leído en la misma transacción. El `SELECT` previo lleva `deleted_at IS NULL AND (?owner IS NULL OR agent_id = ?owner)`: si no encuentra la fila, la función devuelve `Ok(None)` sin escribir nada, así "no existe", "borrada" y "de otro agente" siguen siendo la misma respuesta (`specs/visibility` R4).

Devuelve `Option<UpdatedRow { id, kind, importance, memory_type: Option<String>, tier }>`, que el daemon serializa junto a `updated: 1`. El cliente exige esos campos (sin `#[serde(default)]`): un daemon viejo responde sólo `updated`, y el puente informa el error en vez de decir cómo quedó una fila que no sabe (riesgo de la propuesta).

## D5. Texto de la herramienta

`format_update(&Updated) -> String` en `tools.rs`, puro como el resto del módulo: `updated #12 memory: user_preference, importance 8.0, tier 1`. Para `episodic` y `procedural` sin el tipo. El tier va porque es lo único que el agente no puede deducir del pedido, y porque explica por qué la fila dejó de estar en el workspace.

La descripción del esquema de `memory_type` reusa `MEMORY_TYPE_VALUES` (`tools.rs:31`), igual que `save_memory`.

## D6. Orden de los commits

1. La regla del tier (`tier.rs`), pura.
2. `MemoryPatch` y `update_memory_core` (`admin.rs`), con los tests de R1, R3 y R6.
3. HTTP y cliente.
4. MCP: la herramienta, el texto del refuerzo, la descripción de `save_memory` y los conteos.
5. Docs: `CLAUDE.md` (herramientas y API).

Cada uno compila y pasa la suite solo. El 1 no cambia comportamiento todavía: nada llama a `retier` hasta el 2.
