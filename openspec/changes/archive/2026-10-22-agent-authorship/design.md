# agent-authorship — Diseño

Los enfoques de fondo (A a D) están en la [exploración](exploration.md#enfoques-posibles); se eligió A. Aquí van las decisiones de implementación.

## D1. El lector como dato

`crates/clawg-core/src/memory/sharing.rs` suma:

```rust
pub struct Reader { pub agent_id: String, pub reach: Reach }
impl Reader { pub fn admits(&self, author: Option<&str>) -> bool }
pub fn reader_in(base_dir: &Path, agent_id: &str) -> Reader
```

`reader_in` lee la política y, si no puede, deja al agente con lo suyo y lo que no tiene autor (`Reach::Own`) con un `warn`, que es lo que ya hacía `reach()` del daemon (`crates/clawg-cli/src/daemon/api.rs`). El daemon y el worker usan esta única función.

Cada núcleo que tomaba `owner: Option<&str>` toma `reader: Option<&Reader>`. Los que filtran en Rust (`read_save_core`, LAST SESSION, ACTIVE CONFLICTS) llaman a `Reader::admits`. Los que filtran en SQL bindean el objeto JSON de `reach_bind` (`crates/clawg-core/src/memory/search.rs`), o NULL sin lector, y usan un predicado nuevo, `reader_predicate!`, que es `(?N IS NULL OR <reach_predicate!>)`. Así el predicado sigue escrito una vez y el test que lo compara con `admits` (`the_reach_predicate_agrees_with_admits`) lo sigue cubriendo.

| Alternativa | Por qué no |
|---|---|
| Pasar sólo el `agent_id` y leer la política en cada núcleo | El núcleo de `clawg-core` no conoce el directorio de datos en todos los caminos (el store en memoria de los tests), y cada lectura tendría su propio manejo del error |
| Un `enum Caller { User, Agent(Reader) }` | Es `Option<&Reader>` con otro nombre; `None` ya significa «el usuario» en cada núcleo de hoy |

## D2. `Reach::OwnOnly` desaparece

Era el alcance del detector. Con D4 el detector usa el del que guarda, y nadie más lo usa. `reach_bind` y `Reach::admits` pierden su rama; `format_sharing` también.

## D3. El worker lee la política

`MemoryStore` suma `sharing_dir` (`crates/clawg-core/src/memory/db.rs`): el directorio de datos al abrir el store, y en el store en memoria el que el runtime de prueba le da (`with_sharing_dir`), el mismo `base_dir` del que lee el daemon. No reusa `dir`, que dice dónde abrir una segunda conexión y que el store en memoria no tiene. El worker, una vez por evento en `process_tracked`, arma el `Reader` del que guarda con `MemoryStore::reader_for`; sin directorio usa la política vacía (todo compartido), que es lo que un test espera por defecto. Leer `sharing.json` por save es lo que el daemon ya hace por búsqueda: un archivo chico, leído sin bloqueo.

Una reposición después de un reinicio usa la política de ese momento, no la del momento en que se aceptó el save: la política es de lectura, y el save todavía no se comparó con nada.

## D4. Qué alcance usa cada regla

| Regla | Dónde | Alcance |
|---|---|---|
| Dedup al guardar | `REINFORCE_LOOKUP_SQL`, `store_insert.rs` (refuerzo y `insert_unless_twin`) | proyecto + hash, autor al alcance del que guarda |
| Gemela al restaurar | `RESTORE_TWIN`, `admin.rs` | proyecto + hash, autor al alcance del lector del restore |
| Detector al guardar | `contradiction_search_scope`, `contradiction.rs` | predicado de proyecto, alcance del que guarda |
| Detector al restaurar | `restore_candidates`, `admin.rs` | igual, con el lector del restore; sin lector, toda fila |
| Clave al guardar | `TopicScope`, `SCOPE_ROWS_SQL`, `topic.rs` | proyecto + clave, autor al alcance del que guarda |
| Titular al restaurar | `RESTORE_TOPIC_HOLDER`, `admin.rs` | proyecto + clave, autor al alcance del lector |
| Revivir al borrar | `REPLACED_SQL`, `KEY_HELD_SQL`, `revival.rs` | proyecto + clave, sin autor |
| Import | `TWIN_SQL`, `TOPIC_HOLDERS_SQL`, `import.rs` | proyecto + hash / clave, sin autor (es del usuario) |
| Síntesis | `PENDING`, `Scope`, `synthesis.rs` | proyecto, autor al alcance del lector; cubre una conclusión de cualquier autor |

**Revivir sin alcance.** Borrar no siempre tiene lector (`clawg undelete`, el visor), y el revivir sigue cadenas que ya existen (`superseded_by`), así que no decide qué compara un save. Mirar el alcance del que borra haría que una misma fila reviviera o no según quién la borra. El costo: con `hide B A`, B y A pueden tener cada uno un titular de K; borrar el de B no revive su versión anterior mientras viva el de A. Queda en riesgos, y el doctor ya los marca.

**Una conclusión de cualquier autor cubre.** El aviso de síntesis es «hay cosas sin resumir»; una conclusión oculta para el lector igual existe y la ve quien la puede leer. Exigir el alcance haría que el mismo store tenga pendientes distintas según el agente, que es lo que el cambio quita.

## D5. V20

```sql
DROP INDEX IF EXISTS idx_memory_dedup;
CREATE INDEX IF NOT EXISTS idx_memory_dedup
    ON memory (COALESCE(project_id, ''), content_hash, created_at)
    WHERE deleted_at IS NULL AND content_hash IS NOT NULL;
DROP INDEX IF EXISTS idx_memory_topic;
CREATE INDEX IF NOT EXISTS idx_memory_topic
    ON memory (COALESCE(project_id, ''), topic_key)
    WHERE deleted_at IS NULL AND topic_key IS NOT NULL;
```

El predicado de alcance queda como filtro residual sobre `agent_id`: el índice lleva a las filas del proyecto con ese hash o esa clave, que son pocas. `reinforce_lookup_uses_dedup_index` sigue leyendo la misma constante que producción y suma un plan para la clave. Las alternativas (sin migración, o sumar índices sin quitar los viejos) están en la [exploración](exploration.md#hace-falta-una-migración).

## D6. Síntesis por proyecto

`SynthesisParams` cambia `agent_id` por `reader: Option<Reader>`. El filtro del alcance usa `reader_predicate!` sobre `agent_id`. La revisión se escribe y se lee con `agent_id = ''` y el proyecto como `scope_key`, la clave que ya usaba el modo sin agente; las filas de V10 por agente quedan, sin lectores.

## D7. Pares por proyecto

`open_pairs(store, reader, project_id, limit)`: la fila nueva cumple `memory_project_predicate!("n.", ?)` y los dos lados el predicado del lector. `DisputesQuery` suma `project_id`; el puente lo manda desde `memory_status` con el proyecto del puente. Un daemon anterior responde 400 a un `project_id` desconocido (`deny_unknown_fields`): el puente ya deja fuera la línea cuando la llamada falla.

ACTIVE CONFLICTS lee `agent_id` de los dos lados y filtra con `Reader::admits`; sigue limitado a los pares con un lado entre las filas del prompt.

## D8. El listado

`ListParams` suma `reader: Option<Reader>`; `ListQuery` suma `reader` (el nombre del agente; el daemon arma su alcance). `agent` sigue siendo el filtro de autor del visor. El puente manda `reader` siempre y `agent` sólo cuando el modelo lo pide. Un daemon anterior responde 400 a `reader`: el puente lo explica como cualquier fallo de la llamada, y `version_warning` ya avisa del daemon viejo en el visor.

`format_rows` recibe el lector y agrega ` by <autor>` (con `display_author`) a una fila ajena, después del texto de estado y antes de la clave, separado de ` by=#N`, que siempre lleva `=`.

## D9. Lo que se descarta en este cambio

- Que el puente sugiera `clawg stop` ante un 404 sobre una fila ajena (un daemon anterior con un puente nuevo): el 404 de un daemon anterior dice lo mismo que el de una fila borrada, y `clawg update` reemplaza los dos juntos.
- Nombrar al autor en la línea de `replaced`: la respuesta ya da el id y la primera línea, y `read_memory` lo nombra.
